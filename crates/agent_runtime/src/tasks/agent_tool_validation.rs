use crate::ResourceContext;
use crate::runtime::RuntimeServices;
use crate::runtime::{TaskKind, ToolExecutionMode};
use crate::tasks::delegate_task::delegate_task_spec;
use crate::tasks::update_plan::update_plan_spec;
use crate::tools::{ToolName, ToolSpec};

pub(super) fn specs_for_task(
    kind: TaskKind,
    mode: ToolExecutionMode,
    services: &RuntimeServices,
    resources: &ResourceContext,
) -> Vec<ToolSpec> {
    if kind == TaskKind::Ask {
        return Vec::new();
    }

    let mut specs = services.tools.specs(resources);
    if mode == ToolExecutionMode::ReadOnly {
        specs.retain(|spec| spec.risk == crate::risk::RiskLevel::Read);
    }
    specs.push(update_plan_spec());
    specs.push(delegate_task_spec());
    specs
}

pub(super) fn tool_is_available(specs: &[ToolSpec], name: &ToolName) -> bool {
    specs.iter().any(|spec| &spec.name == name)
}

pub(super) fn available_tool_names(specs: &[ToolSpec]) -> String {
    if specs.is_empty() {
        return "无".into();
    }
    specs
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 给模型返回的原始 function name 生成可诊断展示值。
///
/// [`ToolName`] 会为了 provider 兼容性做规范化，错误信息必须使用原始字段，
/// 否则空名/非法名都会被误报成规范化后的工具名。
pub(super) fn raw_tool_name_for_display(name: &str) -> String {
    if name.trim().is_empty() {
        "<empty>".to_string()
    } else {
        name.to_string()
    }
}

pub(super) fn raw_tool_name_is_empty(name: &str) -> bool {
    name.trim().is_empty()
}

pub(super) fn malformed_tool_call_reason(
    llm_call: &llm_connector::types::ToolCall,
    err: &crate::error::ToolError,
) -> String {
    let args = llm_call.function.arguments.trim();
    let preview: String = args.chars().take(120).collect();
    format!(
        "模型返回了无效工具调用 `{}`: {err}。arguments 必须是 JSON object,当前开头为 `{preview}`。",
        raw_tool_name_for_display(&llm_call.function.name)
    )
}
