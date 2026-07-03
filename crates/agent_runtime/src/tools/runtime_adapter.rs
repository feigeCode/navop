//! Adapters between agent_runtime compatibility types and tool_runtime core types.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::error::ToolError;
use crate::resource::ResourceId;
use crate::tools::{ObservationData, Tool, ToolName, ToolObservation, ToolRegistry, ToolSpec};
use crate::{ResourceContext, SessionId, ToolCall, ToolExecutionMode, TurnId};

const PROVIDER_TARGET_FIELDS: [&str; 3] = ["connection", "connection_id", "session_id"];

pub fn runtime_descriptors_to_specs(
    descriptors: &[tool_runtime::RuntimeToolDescriptor],
) -> Vec<crate::tools::ToolSpec> {
    descriptors
        .iter()
        .map(agent_spec_from_runtime_descriptor)
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
        agent_spec_from_runtime_descriptor(&self.descriptor)
    }

    fn supports_parallel(&self) -> bool {
        self.descriptor.annotations.supports_parallel
    }

    async fn execute(
        &self,
        mut invocation: crate::tools::ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let (arguments, resource_id) = normalize_agent_arguments(
            &self.descriptor,
            invocation.arguments.clone(),
            &invocation.resources,
            invocation.resource_id.clone(),
        )?;
        invocation.resource_id = invocation.resource_id.or(resource_id);
        let result = self
            .registry
            .call(
                &self.runtime_id,
                arguments,
                tool_runtime::ToolContext::for_adapter(self.adapter),
            )
            .await
            .map_err(runtime_tool_error)?;
        Ok(runtime_result_to_observation(invocation, result))
    }
}

fn agent_spec_from_runtime_descriptor(
    descriptor: &tool_runtime::RuntimeToolDescriptor,
) -> ToolSpec {
    let mut spec = ToolSpec::from_runtime_descriptor(descriptor);
    spec.parameters = agent_target_schema(&spec.parameters);
    spec
}

fn agent_target_schema(schema: &Value) -> Value {
    let mut schema = schema.clone();
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };
    let provider_field = provider_target_field(properties);
    if let Some(field) = provider_field {
        let target = properties.get(field).cloned().unwrap_or_else(target_schema);
        remove_provider_target_fields(properties);
        properties.entry("target".to_string()).or_insert(target);
    }
    rewrite_required_targets(&mut schema);
    schema
}

fn normalize_agent_arguments(
    descriptor: &tool_runtime::RuntimeToolDescriptor,
    arguments: Value,
    resources: &ResourceContext,
    explicit_resource: Option<ResourceId>,
) -> Result<(Value, Option<ResourceId>), ToolError> {
    let has_target = descriptor_has_target(descriptor);
    let provider_field = descriptor_provider_target_field(descriptor);
    if !has_target && provider_field.is_none() {
        return Ok((arguments, explicit_resource));
    }
    let Value::Object(mut arguments) = arguments else {
        return Ok((arguments, explicit_resource));
    };
    reject_provider_target_fields(&arguments)?;
    let target = take_string(&mut arguments, "target")?;
    let resource_id = resolve_target_id(
        target.as_deref(),
        resources,
        explicit_resource,
        &descriptor.target,
    )?;
    if has_target {
        if let Some(id) = &resource_id {
            arguments.insert("target".to_string(), Value::String(id.to_string()));
        }
        return Ok((Value::Object(arguments), resource_id));
    }
    let field = provider_field.expect("provider target field should exist");
    if let Some(id) = &resource_id {
        arguments.insert(field.to_string(), Value::String(id.to_string()));
    }
    remove_provider_target_fields_except(&mut arguments, field);
    Ok((Value::Object(arguments), resource_id))
}

fn descriptor_provider_target_field(
    descriptor: &tool_runtime::RuntimeToolDescriptor,
) -> Option<&'static str> {
    descriptor
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(provider_target_field)
}

fn descriptor_has_target(descriptor: &tool_runtime::RuntimeToolDescriptor) -> bool {
    descriptor
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key("target"))
}

fn provider_target_field(properties: &Map<String, Value>) -> Option<&'static str> {
    PROVIDER_TARGET_FIELDS
        .iter()
        .copied()
        .find(|field| properties.contains_key(*field))
}

fn remove_provider_target_fields(properties: &mut Map<String, Value>) {
    for field in PROVIDER_TARGET_FIELDS {
        properties.remove(field);
    }
}

fn remove_provider_target_fields_except(properties: &mut Map<String, Value>, keep: &str) {
    for field in PROVIDER_TARGET_FIELDS {
        if field != keep {
            properties.remove(field);
        }
    }
}

fn reject_provider_target_fields(arguments: &Map<String, Value>) -> Result<(), ToolError> {
    for field in PROVIDER_TARGET_FIELDS {
        if arguments.contains_key(field) {
            return Err(ToolError::InvalidArguments(format!(
                "field `{field}` is not agent-facing; use `target`"
            )));
        }
    }
    Ok(())
}

fn rewrite_required_targets(schema: &mut Value) {
    let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) else {
        return;
    };
    let mut rewritten = Vec::with_capacity(required.len());
    for item in required.drain(..) {
        let item = match item.as_str() {
            Some(field) if PROVIDER_TARGET_FIELDS.contains(&field) => {
                Value::String("target".into())
            }
            _ => item,
        };
        if !rewritten.contains(&item) {
            rewritten.push(item);
        }
    }
    *required = rewritten;
}

fn take_string(arguments: &mut Map<String, Value>, key: &str) -> Result<Option<String>, ToolError> {
    match arguments.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(ToolError::InvalidArguments(format!(
            "field `{key}` must be a string"
        ))),
    }
}

fn resolve_target_id(
    target: Option<&str>,
    resources: &ResourceContext,
    explicit_resource: Option<ResourceId>,
    target_spec: &tool_runtime::ToolTargetSpec,
) -> Result<Option<ResourceId>, ToolError> {
    if let Some(target) = target {
        return resolve_named_target(target, resources, target_spec);
    }
    if let Some(id) = explicit_resource {
        return Ok(Some(id));
    }
    Ok(resources.current().map(|resource| resource.id.clone()))
}

fn resolve_named_target(
    target: &str,
    resources: &ResourceContext,
    target_spec: &tool_runtime::ToolTargetSpec,
) -> Result<Option<ResourceId>, ToolError> {
    if resources.is_empty() {
        return Ok(Some(ResourceId::new(target)));
    }
    let pool = resources.to_runtime_resource_pool();
    pool.resolve_target_for_spec(target, target_spec)
        .map(|resource| Some(ResourceId::new(resource.id.as_str())))
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))
}

fn target_schema() -> Value {
    serde_json::json!({
        "type": "string",
        "description": "Target resource id, label, or alias from the resource pool."
    })
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
