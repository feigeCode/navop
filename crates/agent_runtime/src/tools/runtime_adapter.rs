//! Adapters between agent_runtime compatibility types and tool_runtime core types.

use async_trait::async_trait;

use crate::error::ToolError;
use crate::tools::{ObservationData, Tool, ToolName, ToolObservation, ToolRegistry, ToolSpec};
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

pub fn tool_runtime_agent_tool_registry(
    registry: tool_runtime::ToolRegistry,
    adapter: tool_runtime::ToolAdapter,
) -> ToolRegistry {
    let mut agent_registry = ToolRegistry::new();
    for descriptor in registry.list_runtime(adapter) {
        agent_registry.register(std::sync::Arc::new(ToolRuntimeAgentTool {
            name: ToolName::new(descriptor.id.as_str()),
            runtime_id: descriptor.id.as_str().to_string(),
            descriptor,
            registry: registry.clone(),
            adapter,
        }));
    }
    agent_registry
}

struct ToolRuntimeAgentTool {
    name: ToolName,
    runtime_id: String,
    descriptor: tool_runtime::RuntimeToolDescriptor,
    registry: tool_runtime::ToolRegistry,
    adapter: tool_runtime::ToolAdapter,
}

#[async_trait]
impl Tool for ToolRuntimeAgentTool {
    fn name(&self) -> ToolName {
        self.name.clone()
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::from_runtime_descriptor(&self.descriptor)
    }

    fn supports_parallel(&self) -> bool {
        self.descriptor.annotations.supports_parallel
    }

    async fn execute(
        &self,
        invocation: crate::tools::ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let result = self
            .registry
            .call(
                &self.runtime_id,
                invocation.arguments.clone(),
                tool_runtime::ToolContext::for_adapter(self.adapter),
            )
            .await
            .map_err(runtime_tool_error)?;
        Ok(runtime_result_to_observation(invocation, result))
    }
}

fn runtime_tool_error(error: tool_runtime::ToolError) -> ToolError {
    match error {
        tool_runtime::ToolError::UnknownTool { id } => ToolError::NotFound(id),
        tool_runtime::ToolError::UnsupportedAdapter { id, adapter } => ToolError::Execution(
            format!("tool `{id}` is not exposed for adapter {adapter:?}"),
        ),
        tool_runtime::ToolError::Failed { message } => ToolError::Execution(message),
    }
}

fn runtime_result_to_observation(
    invocation: crate::tools::ToolInvocation,
    result: tool_runtime::ToolResult,
) -> ToolObservation {
    let data = ObservationData::Json(result.structured_content);
    let summary = data.to_text();
    let summary = if summary.trim().is_empty() {
        "Tool succeeded".to_string()
    } else {
        summary
    };
    ToolObservation::success(invocation.call_id, invocation.tool_name, summary, data)
        .with_resource(invocation.resource_id)
}
