use crate::ResourceContext;
use crate::runtime::RuntimeServices;
use crate::tasks::update_plan::{UPDATE_PLAN_TOOL, update_plan_spec};
use crate::tools::{ToolName, ToolSpec};

pub(super) fn services_with_update_plan_specs(
    services: &RuntimeServices,
    resources: &ResourceContext,
) -> Vec<ToolSpec> {
    let mut specs = services.tools.specs(resources);
    specs.push(update_plan_spec());
    specs
}

pub(super) fn tool_is_available(
    services: &RuntimeServices,
    resources: &ResourceContext,
    name: &ToolName,
) -> bool {
    name.as_str() == UPDATE_PLAN_TOOL
        || services
            .tools
            .specs(resources)
            .iter()
            .any(|spec| &spec.name == name)
}

pub(super) fn malformed_tool_call_reason(
    llm_call: &llm_connector::types::ToolCall,
    err: &crate::error::ToolError,
) -> String {
    let args = llm_call.function.arguments.trim();
    let preview: String = args.chars().take(120).collect();
    format!(
        "模型返回了无效工具调用 `{}`: {err}。arguments 必须是 JSON object,当前开头为 `{preview}`。",
        ToolName::new(llm_call.function.name.clone())
    )
}
