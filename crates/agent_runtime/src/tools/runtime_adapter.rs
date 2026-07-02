//! Adapters between agent_runtime compatibility types and tool_runtime core types.

use crate::{ResourceContext, SessionId, ToolCall, ToolExecutionMode, TurnId};

pub fn runtime_descriptors_to_specs(
    descriptors: &[tool_runtime::RuntimeToolDescriptor],
) -> Vec<crate::tools::ToolSpec> {
    descriptors
        .iter()
        .map(crate::tools::ToolSpec::from_runtime_descriptor)
        .collect()
}

pub fn permission_policy_for_tool_mode(mode: ToolExecutionMode) -> tool_runtime::PermissionPolicy {
    let profile = match mode {
        ToolExecutionMode::ReadOnly => tool_runtime::PermissionProfile::Safe,
        ToolExecutionMode::Manual => tool_runtime::PermissionProfile::Confirm,
        ToolExecutionMode::Auto => tool_runtime::PermissionProfile::Auto,
    };
    tool_runtime::PermissionPolicy::for_profile(profile)
}

pub fn runtime_tool_invocation_from_call(
    call: &ToolCall,
    resources: &ResourceContext,
    tool_mode: ToolExecutionMode,
    session_id: SessionId,
    turn_id: TurnId,
) -> tool_runtime::ToolInvocation {
    tool_runtime::ToolInvocation::new(
        tool_runtime::ToolId::new(call.tool_name.as_str()),
        call.arguments.clone(),
        resources.to_runtime_resource_pool(),
        permission_policy_for_tool_mode(tool_mode),
        tool_runtime::ToolCaller::Agent,
    )
    .with_audit(tool_runtime::AuditContext {
        session_id: Some(session_id.to_string()),
        turn_id: Some(turn_id.to_string()),
        request_id: Some(call.call_id.to_string()),
    })
}
