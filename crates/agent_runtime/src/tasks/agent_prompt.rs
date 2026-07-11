use crate::runtime::TaskKind;
use crate::skill::SkillContext;
use crate::tasks::skill_prompt::append_skill_context;
use crate::tools::ToolSpec;
use crate::{Plan, ResourceContext};

const AGENT_SYSTEM: &str = "你是 Navop 的 AI 运维助手。请根据用户目标自主决定如何行动:\
简单问题直接、简洁地用简体中文回答;需要查询或操作资源时调用相应工具;\
面对多步任务时,先调用 `update_plan` 列出步骤并随进展更新状态(每完成一步就更新)。\
可用 `delegate_task` 将边界清晰的子任务交给隔离子代理执行;它不是后端或 Codex CLI 选择。\
不要为简单问题强行制定计划。完成后直接给出最终回答。";

const ASK_SYSTEM: &str = "你是 Navop 的 AI 助手。当前处于 Ask 模式:\
只直接、简洁地回答用户问题;不要创建计划;不要调用任何工具。\
如果用户需要查询、操作或使用上下文资源,请提示切换到 Agent 或 Plan 模式。\
回答使用简体中文。";

const PLAN_SYSTEM: &str = "你是 Navop 的 AI 助手。当前处于 Plan 模式:\
面对用户目标时先调用 `update_plan` 给出清晰步骤,再按步骤执行;每完成一步都更新计划状态。\
可用 `delegate_task` 将边界清晰的子任务交给隔离子代理执行;它不是后端或 Codex CLI 选择。\
如果目标缺少必要信息,先提出需要补充的问题。回答使用简体中文。";

pub(super) fn build_system_prompt(
    kind: TaskKind,
    tools: &[ToolSpec],
    resources: &ResourceContext,
    skills: &SkillContext,
    system_instruction: Option<&str>,
    current_plan: Option<&Plan>,
) -> String {
    let mut prompt = system_prompt(kind).to_string();
    append_system_instruction(&mut prompt, system_instruction);
    append_resource_context(&mut prompt, resources);
    append_skill_context(&mut prompt, skills);
    if let Some(plan) = current_plan {
        append_current_plan(&mut prompt, plan);
    }
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
        append_terminal_tool_selection_rules(&mut prompt, tools);
        append_canonical_runtime_tool_rules(&mut prompt, tools);
    }
    prompt
}

fn append_terminal_tool_selection_rules(prompt: &mut String, tools: &[ToolSpec]) {
    let terminal_exec = find_tool_name(tools, &["terminal_exec", "terminal.exec"]);
    let terminal_control = find_tool_name(tools, &["terminal_control", "terminal.control"]);
    let ssh_exec = find_tool_name(tools, &["ssh_exec", "ssh.exec", "ssh_remote_exec"]);
    if terminal_exec.is_none() && terminal_control.is_none() && ssh_exec.is_none() {
        return;
    }

    prompt.push_str("\n\n终端/SSH 工具选择规则:");
    if let Some(name) = terminal_exec {
        prompt.push_str(&format!(
            " 当用户明确要求在可见终端、当前终端、右侧终端、像手动输入一样执行，或说“就在这个终端里执行”时，优先调用 `{name}`；`command` 必须保持用户要输入的命令文本，`target` 必须指向终端资源，通常设置 `submit=true`。`{name}` 会写入 live terminal，不要声称有 exit code，除非工具观测结果明确返回 exit_code。"
        ));
    }
    if let Some(name) = ssh_exec {
        prompt.push_str(&format!(
            " 当用户只要求后台/结构化 SSH 命令执行、收集 stdout/stderr 或非交互检查时，使用 `{name}`；如果用户要求可见终端执行且可用工具里有终端执行工具，不要用 `{name}` 替代。"
        ));
    }
    if let Some(name) = terminal_control {
        prompt.push_str(&format!(
            " 当用户明确要求停止、打断当前可见终端的前台任务或发送 Ctrl+C 时，调用 `{name}` 并设置 `action=interrupt`；只有工具结果明确返回 `sent=true` 后才能声称已发送 Ctrl+C。不要把 `\\u0003` 作为 `terminal_exec` 的 command；Agent 取消对话不会中断终端任务。"
        ));
    }
}

fn append_canonical_runtime_tool_rules(prompt: &mut String, tools: &[ToolSpec]) {
    let rules = canonical_runtime_tool_rules(tools);
    if rules.is_empty() {
        return;
    }
    prompt.push_str("\n\n统一工具命名规则: ");
    prompt.push_str(&rules.join(" "));
}

fn canonical_runtime_tool_rules(tools: &[ToolSpec]) -> Vec<String> {
    let mut rules = Vec::new();
    if let Some(name) = find_tool_name(tools, &["db_exec", "db.exec"]) {
        rules.push(format!("数据库写入使用 `{name}`。"));
    }
    append_family_rule(
        &mut rules,
        tools,
        CanonicalFamilyRule::new(
            "SFTP 文件操作",
            &[
                "sftp_list",
                "sftp_read",
                "sftp_write",
                "sftp_stat",
                "sftp_upload",
                "sftp_download",
            ],
        ),
    );
    append_family_rule(
        &mut rules,
        tools,
        CanonicalFamilyRule::new(
            "Redis 操作",
            &["redis_command", "redis_keys", "redis_get", "redis_set"],
        ),
    );
    rules
}

struct CanonicalFamilyRule<'a> {
    label: &'a str,
    names: &'a [&'a str],
}

impl<'a> CanonicalFamilyRule<'a> {
    fn new(label: &'a str, names: &'a [&'a str]) -> Self {
        Self { label, names }
    }
}

fn append_family_rule(rules: &mut Vec<String>, tools: &[ToolSpec], rule: CanonicalFamilyRule<'_>) {
    let canonical = rule
        .names
        .iter()
        .filter_map(|name| find_tool_name(tools, &[*name]))
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>();
    if canonical.is_empty() {
        return;
    }
    rules.push(format!("{}使用 {}。", rule.label, canonical.join("、")));
}

fn find_tool_name<'a>(tools: &'a [ToolSpec], candidates: &[&str]) -> Option<&'a str> {
    tools.iter().find_map(|tool| {
        let name = tool.name.as_str();
        candidates.contains(&name).then_some(name)
    })
}

fn append_system_instruction(prompt: &mut String, instruction: Option<&str>) {
    let Some(instruction) = instruction.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    prompt.push_str("\n\n用户自定义系统提示:\n");
    prompt.push_str(instruction);
}

fn append_current_plan(prompt: &mut String, plan: &Plan) {
    if plan.steps.is_empty() {
        return;
    }
    prompt.push_str("\n\n当前计划(Todo)状态:\n");
    prompt.push_str(&format!("目标: {}\n", plan.goal));
    prompt.push_str(&plan.describe());
    prompt.push_str(
        "如果用户要求继续、下一步或完成剩余任务,基于此计划推进;\
如步骤状态发生变化,必须调用 `update_plan` 提交完整最新计划,不要把工具调用写成普通文本。",
    );
}

fn append_resource_context(prompt: &mut String, resources: &ResourceContext) {
    if resources.is_empty() {
        return;
    }
    prompt.push_str("\n\n资源池:\n");
    prompt.push_str(&resources.describe());
    prompt.push_str(
        "调用工具时使用上面列出的资源 id、名称或标签作为 target 参数;\
当前标记为 [当前] 的资源是默认目标,但不是资源池边界。\
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::{SkillContext, SkillRef, SkillSummary};

    #[test]
    fn system_prompts_use_navop_brand() {
        for kind in [TaskKind::Agent, TaskKind::Ask, TaskKind::Plan] {
            let prompt = system_prompt(kind);
            assert!(prompt.contains("Navop"));
            assert!(!prompt.contains("onetcli"));
        }
    }

    #[test]
    fn system_prompt_includes_selected_skill_metadata_without_contents() {
        let skills = SkillContext::new().with_skill(SkillRef::new(
            "ops",
            "Run operational playbooks",
            "/tmp/skills/ops/SKILL.md",
        ));

        let prompt = build_system_prompt(
            TaskKind::Agent,
            &[],
            &ResourceContext::new(),
            &skills,
            None,
            None,
        );

        assert!(prompt.contains("Selected skills for this turn"));
        assert!(prompt.contains("ops"));
        assert!(prompt.contains("Run operational playbooks"));
        assert!(prompt.contains("load_skill"));
        assert!(prompt.contains("read_skill_file"));
        assert!(!prompt.contains("Follow the ops checklist."));
        assert!(!prompt.contains("Instructions:"));
    }

    #[test]
    fn system_prompt_includes_available_skill_catalog_metadata() {
        let skills = SkillContext::new().with_available_skill(SkillSummary::new(
            "using-superpowers",
            "Use Superpowers workflows",
            "/tmp/skills/using-superpowers/SKILL.md",
        ));

        let prompt = build_system_prompt(
            TaskKind::Agent,
            &[],
            &ResourceContext::new(),
            &skills,
            None,
            None,
        );

        assert!(prompt.contains("Available skill catalog"));
        assert!(prompt.contains("using-superpowers"));
        assert!(prompt.contains("Use Superpowers workflows"));
        assert!(!prompt.contains("Instructions:"));
    }
}
