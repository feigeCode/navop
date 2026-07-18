use super::{
    PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider,
    target_adapter::{mcp_target_schema, normalize_mcp_arguments},
};
use crate::approval::PublicMcpApprovalOutcome;
use crate::approval_grants::redact_approval_arguments;
use crate::permissions::{PublicMcpOperationKind, permission_policy_for_mode};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool, ToolAnnotations},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, ResourceKind, ResourcePool, ResourceRef, ToolAdapter,
    ToolAnnotations as RuntimeToolAnnotations, ToolContext, ToolDescriptor, ToolError,
    ToolRegistry, ToolResult,
};

pub type ResourcePoolProvider = Arc<dyn Fn() -> Option<ResourcePool> + Send + Sync>;
const CONNECTIONS_LIST_SESSIONS_TOOL: &str = "connections.list_sessions";

#[derive(Clone)]
pub struct ToolRuntimeMcpProvider {
    registry: ToolRegistry,
    resource_pool_provider: Option<ResourcePoolProvider>,
}

impl ToolRuntimeMcpProvider {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            resource_pool_provider: None,
        }
    }

    pub fn with_resource_pool(mut self, resource_pool: ResourcePool) -> Self {
        self.resource_pool_provider = Some(Arc::new(move || Some(resource_pool.clone())));
        self
    }

    pub fn with_resource_pool_provider(mut self, provider: ResourcePoolProvider) -> Self {
        self.resource_pool_provider = Some(provider);
        self
    }
}

impl PublicMcpToolProvider for ToolRuntimeMcpProvider {
    fn tools(&self) -> Vec<Tool> {
        let descriptors = self.registry.list(ToolAdapter::Mcp);
        let has_unified_sessions_tool = descriptors
            .iter()
            .any(|tool| tool.id == CONNECTIONS_LIST_SESSIONS_TOOL);
        let mut tools = descriptors
            .into_iter()
            .map(runtime_tool_to_mcp_tool)
            .collect::<Vec<_>>();
        if self.resource_pool_provider.is_some() && !has_unified_sessions_tool {
            tools.push(resource_sessions_tool());
        }
        tools
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        if name == CONNECTIONS_LIST_SESSIONS_TOOL && self.resource_pool_provider.is_some() {
            return Some(self.call_resource_sessions(arguments));
        }
        let runtime_descriptor = self.registry.get_runtime(name, ToolAdapter::Mcp)?;
        let target_spec = runtime_descriptor.target.clone();
        let descriptor = runtime_descriptor.legacy_descriptor();
        let registry = self.registry.clone();
        let resource_pool = self
            .resource_pool_provider
            .as_ref()
            .and_then(|provider| provider());
        let name = name.to_string();
        let raw_input = Value::Object(arguments.unwrap_or_default());
        let input = match normalize_mcp_arguments(
            &descriptor.input_schema,
            raw_input.clone(),
            resource_pool.as_ref(),
            Some(&target_spec),
        ) {
            Ok(input) => input,
            Err(error) => return Some(Box::pin(async move { Err(error) })),
        };
        Some(Box::pin(async move {
            let call_annotations = registry
                .call_annotations(&name, ToolAdapter::Mcp, &input)
                .unwrap_or_else(|| descriptor.annotations.clone());
            call_runtime_tool(
                registry,
                descriptor,
                call_annotations,
                name,
                raw_input,
                input,
                context,
            )
            .await
        }))
    }
}

impl ToolRuntimeMcpProvider {
    fn call_resource_sessions(&self, arguments: Option<JsonObject>) -> PublicMcpToolFuture {
        let resource_pool = self
            .resource_pool_provider
            .as_ref()
            .and_then(|provider| provider())
            .unwrap_or_default();
        let input = Value::Object(arguments.unwrap_or_default());
        Box::pin(async move { list_resource_sessions(resource_pool, input) })
    }
}

async fn call_runtime_tool(
    registry: ToolRegistry,
    descriptor: ToolDescriptor,
    call_annotations: RuntimeToolAnnotations,
    name: String,
    request_input: Value,
    input: Value,
    context: PublicMcpToolContext,
) -> Result<CallToolResult, McpError> {
    let policy = permission_policy_for_mode(context.permission_mode);
    match policy.decide(&descriptor.tool_id(), None, &call_annotations) {
        tool_runtime::PermissionDecision::Allow => run_runtime_tool(registry, name, input).await,
        tool_runtime::PermissionDecision::Ask => {
            ask_then_run_runtime_tool(registry, descriptor, name, request_input, input, context)
                .await
        }
        tool_runtime::PermissionDecision::Deny => Ok(permission_denied_result(
            "tool runtime call denied by permission mode; set MCP Permission Profile to Confirm or Auto",
        )),
    }
}

async fn ask_then_run_runtime_tool(
    registry: ToolRegistry,
    descriptor: ToolDescriptor,
    name: String,
    request_input: Value,
    input: Value,
    context: PublicMcpToolContext,
) -> Result<CallToolResult, McpError> {
    let outcome = context
        .request_approval(
            PublicMcpOperationKind::CallToolRuntimeTool,
            name.clone(),
            format!("Call {}", descriptor.title),
            json!({
                "tool": name,
                "requestArguments": redact_approval_arguments(request_input),
                "arguments": redact_approval_arguments(input.clone()),
            }),
        )
        .await;

    match outcome {
        PublicMcpApprovalOutcome::Approved => run_runtime_tool(registry, name, input).await,
        PublicMcpApprovalOutcome::Denied { reason } => {
            Ok(permission_denied_result(reason.unwrap_or_else(|| {
                "tool runtime call denied by approval".to_string()
            })))
        }
    }
}

async fn run_runtime_tool(
    registry: ToolRegistry,
    name: String,
    input: Value,
) -> Result<CallToolResult, McpError> {
    registry
        .call(&name, input, ToolContext::for_adapter(ToolAdapter::Mcp))
        .await
        .map(runtime_result_to_mcp_result)
        .map_err(runtime_error_to_mcp_error)
}

fn runtime_tool_to_mcp_tool(descriptor: ToolDescriptor) -> Tool {
    Tool::new(
        descriptor.id,
        descriptor.description,
        schema_object(mcp_target_schema(descriptor.input_schema)),
    )
    .with_annotations(runtime_annotations_to_mcp_annotations(
        descriptor.annotations,
    ))
}

fn resource_sessions_tool() -> Tool {
    Tool::new(
        CONNECTIONS_LIST_SESSIONS_TOOL,
        "List saved and active Navop connection sessions from the current resource pool. Pass kind or connection_type to filter; omit filters to return all available resources.",
        schema_object(resource_sessions_schema()),
    )
    .with_annotations(
        ToolAnnotations::with_title("List connection sessions")
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
    )
}

fn resource_sessions_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {
                "type": "string",
                "description": "Resource or connection type filter, for example database, ssh_sftp, redis, mongodb, terminal, mysql, postgres, or sqlite."
            },
            "connection_type": {
                "type": "string",
                "description": "Alias of kind. Omit both filters to list all resources."
            }
        }
    })
}

fn list_resource_sessions(
    resource_pool: ResourcePool,
    input: Value,
) -> Result<CallToolResult, McpError> {
    let kind = resource_session_kind_filter(&input)?;
    let sessions = resource_pool
        .resources
        .into_iter()
        .filter(|resource| resource_matches_session_kind(resource, kind.as_deref()))
        .map(resource_session_value)
        .collect::<Vec<_>>();
    let total = sessions.len();
    Ok(CallToolResult::structured(json!({
        "sessions": sessions,
        "total": total,
        "kind": kind
    })))
}

fn resource_session_value(resource: ResourceRef) -> Value {
    let id = resource.id.as_str().to_string();
    let kind = resource.kind.as_str().to_string();
    json!({
        "id": id,
        "label": resource.label,
        "kind": kind,
        "aliases": resource.aliases,
        "scopes": resource.scopes,
        "capabilities": resource.capabilities,
        "origin": resource.origin
    })
}

fn resource_session_kind_filter(input: &Value) -> Result<Option<String>, McpError> {
    for field in ["kind", "connection_type"] {
        match input.get(field) {
            Some(Value::String(value)) if !value.trim().is_empty() => {
                return Ok(Some(value.trim().to_ascii_lowercase()));
            }
            Some(Value::String(_)) | Some(Value::Null) | None => {}
            Some(_) => {
                return Err(McpError::invalid_params(
                    format!("field `{field}` must be a string"),
                    None,
                ));
            }
        }
    }
    Ok(None)
}

fn resource_matches_session_kind(resource: &ResourceRef, kind: Option<&str>) -> bool {
    let Some(kind) = kind else {
        return true;
    };
    match kind {
        "all" => true,
        "database" | "db" => is_database_resource(&resource.kind),
        "ssh" | "ssh_sftp" => {
            matches!(resource.kind, ResourceKind::Ssh | ResourceKind::Sftp)
                || resource
                    .capabilities
                    .contains(&ResourceCapability::RemoteExec)
        }
        "mongodb" | "mongo" => matches!(resource.kind, ResourceKind::Mongo),
        "terminal" | "local" | "serial" => matches!(resource.kind, ResourceKind::Terminal),
        other => resource.kind.as_str() == other,
    }
}

fn is_database_resource(kind: &ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Mysql | ResourceKind::Postgres | ResourceKind::Sqlite
    ) || kind.as_str() == "database"
}

fn schema_object(schema: Value) -> Arc<JsonObject> {
    match schema {
        Value::Object(object) => Arc::new(object),
        _ => {
            let mut object = JsonObject::new();
            object.insert("type".to_string(), Value::String("object".to_string()));
            Arc::new(object)
        }
    }
}

fn runtime_annotations_to_mcp_annotations(
    annotations: tool_runtime::ToolAnnotations,
) -> ToolAnnotations {
    ToolAnnotations::with_title(annotations.title)
        .read_only(annotations.read_only)
        .destructive(annotations.destructive)
        .idempotent(annotations.idempotent)
        .open_world(annotations.open_world)
}

fn runtime_result_to_mcp_result(result: ToolResult) -> CallToolResult {
    CallToolResult::structured(result.structured_content)
}

fn runtime_error_to_mcp_error(error: ToolError) -> McpError {
    match error {
        ToolError::UnknownTool { id } => {
            McpError::invalid_params(format!("unknown tool: {id}"), None)
        }
        ToolError::UnsupportedAdapter { id, adapter } => McpError::invalid_params(
            format!("tool `{id}` is not exposed for adapter {adapter:?}"),
            None,
        ),
        ToolError::Failed { message } => McpError::internal_error(message, None),
    }
}

fn permission_denied_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "code": "permission_denied",
        "message": message.into()
    }))
}
