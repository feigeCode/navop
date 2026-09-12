use chrono::Local;
use rust_i18n::t;

use crate::skill::SkillContext;
use crate::tasks::skill_prompt::skill_section;
use crate::tools::ToolSpec;
use crate::{Plan, ResourceContext};

/// 组装内置 Agent 的系统提示词。
///
/// 静态行为规则统一为一份本地化模板（[`AgentRuntime.system_prompt`]），动态上下文
/// 通过占位符注入：
///
/// - `{{resource_info}}` 资源池
/// - `{{skill_info}}` Skill 元数据与使用规则
/// - `{{plan_info}}` 当前计划（Todo）
/// - `{{tool_info}}` 可用工具名与工具选择规则
/// - `{{current_time}}` 当前本地时间
///
/// 该模板固定，用户不可删除。`custom_instruction` 是设置里的「自定义部分」，
/// 始终追加在模板之后（旧配置的追加语义），不会覆盖默认规则与上下文段落。
pub(super) fn build_system_prompt(
    tools: &[ToolSpec],
    resources: &ResourceContext,
    skills: &SkillContext,
    custom_instruction: Option<&str>,
    current_plan: Option<&Plan>,
) -> String {
    let mut prompt = t!("AgentRuntime.system_prompt")
        .to_string()
        .replace("{{resource_info}}", &resource_section(resources))
        .replace("{{skill_info}}", &skill_section(skills))
        .replace("{{plan_info}}", &plan_section(current_plan))
        .replace("{{tool_info}}", &tool_section(tools))
        .replace("{{current_time}}", &current_time_section());

    append_system_instruction(&mut prompt, custom_instruction);
    prompt
}

fn resource_section(resources: &ResourceContext) -> String {
    if resources.is_empty() {
        return String::new();
    }
    t!(
        "AgentRuntime.system_prompt_resource_section",
        resources = resources.describe()
    )
    .to_string()
}

fn plan_section(plan: Option<&Plan>) -> String {
    let Some(plan) = plan.filter(|plan| !plan.steps.is_empty()) else {
        return String::new();
    };
    let plan_text = format!(
        "{}: {}\n{}",
        t!("AgentRuntime.system_prompt_plan_goal_label"),
        plan.goal,
        plan.describe()
    );
    t!("AgentRuntime.system_prompt_plan_section", plan = plan_text).to_string()
}

fn current_time_section() -> String {
    let time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    t!("AgentRuntime.system_prompt_current_time", time = time).to_string()
}

fn tool_section(tools: &[ToolSpec]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut section = t!("AgentRuntime.system_prompt_tool_section", names = names).to_string();
    section.push_str(&terminal_tool_selection_rules(tools));
    section.push_str(&canonical_runtime_tool_rules(tools));
    section
}

fn terminal_tool_selection_rules(tools: &[ToolSpec]) -> String {
    let terminal_exec = find_tool_name(tools, &["terminal_exec", "terminal.exec"]);
    let terminal_read = find_tool_name(tools, &["terminal_read", "terminal.read"]);
    let terminal_control = find_tool_name(tools, &["terminal_control", "terminal.control"]);
    let terminal_write_keys =
        find_tool_name(tools, &["terminal_write_keys", "terminal.write_keys"]);
    let ssh_exec = find_tool_name(tools, &["ssh_exec", "ssh.exec", "ssh_remote_exec"]);

    let mut body = String::new();
    if let Some(name) = ssh_exec {
        body.push_str(&t!("AgentRuntime.tool_rule_ssh_exec", name = name));
    }
    if let Some(name) = terminal_exec {
        let ssh_name = ssh_exec.unwrap_or("ssh.exec");
        body.push_str(&t!(
            "AgentRuntime.tool_rule_terminal_exec",
            name = name,
            ssh_name = ssh_name
        ));
    }
    if let Some(name) = terminal_read {
        body.push_str(&t!("AgentRuntime.tool_rule_terminal_read", name = name));
    }
    if let Some(name) = terminal_control {
        body.push_str(&t!("AgentRuntime.tool_rule_terminal_control", name = name));
    }
    if let Some(name) = terminal_write_keys {
        let control_name = terminal_control.unwrap_or("terminal.control");
        let read_name = terminal_read.unwrap_or("terminal.read");
        body.push_str(&t!(
            "AgentRuntime.tool_rule_terminal_write_keys",
            name = name,
            control_name = control_name,
            read_name = read_name
        ));
    }
    if body.is_empty() {
        return String::new();
    }
    let mut section = t!("AgentRuntime.tool_rule_terminal_header").to_string();
    section.push_str(&body);
    section
}

fn canonical_runtime_tool_rules(tools: &[ToolSpec]) -> String {
    let mut rules: Vec<String> = Vec::new();
    if let Some(name) = find_tool_name(tools, &["db_exec", "db.exec"]) {
        rules.push(t!("AgentRuntime.tool_rule_db_exec", name = name).to_string());
    }
    append_family_rule(
        &mut rules,
        tools,
        t!("AgentRuntime.tool_family_sftp").to_string(),
        &[
            "sftp_list",
            "sftp_read",
            "sftp_write",
            "sftp_stat",
            "sftp_upload",
            "sftp_download",
        ],
    );
    append_family_rule(
        &mut rules,
        tools,
        t!("AgentRuntime.tool_family_redis").to_string(),
        &["redis_command", "redis_keys", "redis_get", "redis_set"],
    );
    if rules.is_empty() {
        return String::new();
    }
    let mut section = t!("AgentRuntime.tool_rule_canonical_header").to_string();
    section.push_str(&rules.join(" "));
    section
}

fn append_family_rule(rules: &mut Vec<String>, tools: &[ToolSpec], label: String, names: &[&str]) {
    let canonical = names
        .iter()
        .filter_map(|name| find_tool_name(tools, &[*name]))
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>();
    if canonical.is_empty() {
        return;
    }
    rules.push(
        t!(
            "AgentRuntime.tool_rule_family",
            label = label,
            names = canonical.join("、")
        )
        .to_string(),
    );
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
    prompt.push_str(&t!(
        "AgentRuntime.system_prompt_extra_instruction",
        instruction = instruction
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::{SkillContext, SkillRef, SkillSummary};
    use crate::{PlanSource, PlanStep, ResourceKind, ResourceRef};

    fn tool_specs(names: &[&str]) -> Vec<ToolSpec> {
        names
            .iter()
            .map(|name| ToolSpec::new(*name, "", serde_json::json!({ "type": "object" })))
            .collect()
    }

    #[test]
    fn default_system_prompt_uses_navop_brand() {
        let prompt = build_system_prompt(
            &[],
            &ResourceContext::new(),
            &SkillContext::new(),
            None,
            None,
        );
        assert!(prompt.contains("Navop"));
        assert!(!prompt.contains("onetcli"));
    }

    #[test]
    fn default_template_keeps_dynamic_context_without_custom_instruction() {
        let resources = ResourceContext::new().with_resource(ResourceRef::new(
            "db-1",
            ResourceKind::Mysql,
            "prod-db",
        ));
        let prompt = build_system_prompt(&[], &resources, &SkillContext::new(), None, None);
        assert!(prompt.contains("Navop"));
        assert!(prompt.contains("prod-db"));
    }

    #[test]
    fn custom_instruction_is_appended_and_does_not_replace_defaults() {
        let resources = ResourceContext::new().with_resource(ResourceRef::new(
            "db-1",
            ResourceKind::Mysql,
            "prod-db",
        ));
        let prompt = build_system_prompt(
            &[],
            &resources,
            &SkillContext::new(),
            Some("始终用 DBA 视角回答。"),
            None,
        );
        assert!(prompt.contains("Navop"));
        assert!(prompt.contains("prod-db"));
        assert!(prompt.contains("始终用 DBA 视角回答。"));
    }

    #[test]
    fn system_prompt_includes_selected_skill_metadata_without_contents() {
        let skills = SkillContext::new().with_skill(SkillRef::new(
            "ops",
            "Run operational playbooks",
            "/tmp/skills/ops/SKILL.md",
        ));
        let prompt = build_system_prompt(&[], &ResourceContext::new(), &skills, None, None);
        assert!(prompt.contains("Selected skills for this turn"));
        assert!(prompt.contains("ops"));
        assert!(prompt.contains("Run operational playbooks"));
        assert!(prompt.contains("load_skill"));
        assert!(prompt.contains("read_skill_file"));
        assert!(!prompt.contains("Follow the ops checklist."));
    }

    #[test]
    fn system_prompt_includes_available_skill_catalog_metadata() {
        let skills = SkillContext::new().with_available_skill(SkillSummary::new(
            "using-superpowers",
            "Use Superpowers workflows",
            "/tmp/skills/using-superpowers/SKILL.md",
        ));
        let prompt = build_system_prompt(&[], &ResourceContext::new(), &skills, None, None);
        assert!(prompt.contains("Available skill catalog"));
        assert!(prompt.contains("using-superpowers"));
        assert!(prompt.contains("Use Superpowers workflows"));
    }

    #[test]
    fn system_prompt_separates_plan_data_from_executable_commands() {
        let plan = Plan::new("巡检集群", PlanSource::Llm)
            .with_steps(vec![PlanStep::new("风险 read", "读取资源状态")]);
        let tools = tool_specs(&["ssh.exec", "terminal.exec", "terminal.read"]);
        let prompt = build_system_prompt(
            &tools,
            &ResourceContext::new(),
            &SkillContext::new(),
            None,
            Some(&plan),
        );
        assert!(prompt.contains("plan_context"));
        assert!(prompt.contains("ssh_exec"));
        assert!(prompt.contains("terminal_exec"));
    }
}
