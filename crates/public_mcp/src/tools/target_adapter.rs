use rmcp::{ErrorData as McpError, model::JsonObject};
use serde_json::{Map, Value, json};
use tool_runtime::{ResourcePool, TargetResolutionError, ToolTargetSpec};

const PROVIDER_TARGET_FIELDS: [&str; 3] = ["connection", "connection_id", "session_id"];

pub(super) fn mcp_target_schema(schema: Value) -> Value {
    let mut schema = schema;
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };
    if let Some(target) = target_property(properties) {
        remove_provider_target_fields(properties);
        let target = properties.entry("target".to_string()).or_insert(target);
        enrich_target_property(target);
    }
    rewrite_required_targets(&mut schema);
    schema
}

pub(super) fn normalize_mcp_arguments(
    schema: &Value,
    input: Value,
    resource_pool: Option<&ResourcePool>,
    target_spec: Option<&ToolTargetSpec>,
) -> Result<Value, McpError> {
    let Value::Object(mut input) = input else {
        return Ok(input);
    };
    reject_provider_target_fields(&input)?;
    if descriptor_has_target(schema) {
        resolve_target_argument(&mut input, resource_pool, target_spec)?;
        return Ok(Value::Object(input));
    }
    let Some(field) = descriptor_provider_target_field(schema) else {
        return Ok(Value::Object(input));
    };
    if let Some(target) = take_target(&mut input)? {
        input.insert(
            field.to_string(),
            Value::String(resolve_target_value(&target, resource_pool, target_spec)?),
        );
    }
    Ok(Value::Object(input))
}

fn target_property(properties: &Map<String, Value>) -> Option<Value> {
    properties.get("target").cloned().or_else(|| {
        descriptor_provider_target_field_from_props(properties).map(|field| {
            properties
                .get(field)
                .cloned()
                .unwrap_or_else(default_target_property)
        })
    })
}

fn descriptor_has_target(schema: &Value) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key("target"))
}

fn descriptor_provider_target_field(schema: &Value) -> Option<&'static str> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(descriptor_provider_target_field_from_props)
}

fn descriptor_provider_target_field_from_props(
    properties: &Map<String, Value>,
) -> Option<&'static str> {
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

fn reject_provider_target_fields(input: &JsonObject) -> Result<(), McpError> {
    for field in PROVIDER_TARGET_FIELDS {
        if input.contains_key(field) {
            return Err(McpError::invalid_params(
                format!("field `{field}` is not MCP-facing; use `target`"),
                None,
            ));
        }
    }
    Ok(())
}

fn take_target(input: &mut JsonObject) -> Result<Option<String>, McpError> {
    match input.remove("target") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(McpError::invalid_params(
            "field `target` must be a string".to_string(),
            None,
        )),
    }
}

fn resolve_target_argument(
    input: &mut JsonObject,
    resource_pool: Option<&ResourcePool>,
    target_spec: Option<&ToolTargetSpec>,
) -> Result<(), McpError> {
    let Some(target) = input.get("target") else {
        return Ok(());
    };
    let Some(target) = target.as_str() else {
        return Err(McpError::invalid_params(
            "field `target` must be a string".to_string(),
            None,
        ));
    };
    let target = resolve_target_value(target, resource_pool, target_spec)?;
    input.insert("target".to_string(), Value::String(target));
    Ok(())
}

fn resolve_target_value(
    target: &str,
    resource_pool: Option<&ResourcePool>,
    target_spec: Option<&ToolTargetSpec>,
) -> Result<String, McpError> {
    let Some(resource_pool) = resource_pool else {
        return Ok(target.to_string());
    };
    match target_spec {
        Some(spec) => resource_pool.resolve_target_for_spec(target, spec),
        None => resource_pool.resolve_target(target),
    }
    .map(|resource| resource.id.as_str().to_string())
    .map_err(|error| target_resolution_error(error, resource_pool, target_spec))
}

fn target_resolution_error(
    error: TargetResolutionError,
    resource_pool: &ResourcePool,
    target_spec: Option<&ToolTargetSpec>,
) -> McpError {
    let available_targets = compatible_targets(resource_pool, target_spec);
    let recovery = if available_targets.is_empty() {
        "No compatible target is currently available. Open or reconnect the required session, call `connections.list_sessions`, then retry with an exact `id`."
            .to_string()
    } else {
        let candidates = available_targets
            .iter()
            .map(|target| {
                format!(
                    "`{}` ({}, {})",
                    target["id"].as_str().unwrap_or_default(),
                    target["label"].as_str().unwrap_or_default(),
                    target["kind"].as_str().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Available compatible targets: {candidates}. Copy an exact `id` from this list or `connections.list_sessions` and retry."
        )
    };
    let message = format!("{error}. {recovery}");
    McpError::invalid_params(
        message,
        Some(json!({
            "error": "target_resolution",
            "available_targets": available_targets,
            "recovery": recovery,
        })),
    )
}

fn compatible_targets(
    resource_pool: &ResourcePool,
    target_spec: Option<&ToolTargetSpec>,
) -> Vec<Value> {
    resource_pool
        .resources
        .iter()
        .filter(|resource| {
            target_spec.is_none_or(|spec| {
                (spec.supported_kinds.is_empty() || spec.supported_kinds.contains(&resource.kind))
                    && spec
                        .required_capabilities
                        .iter()
                        .all(|capability| resource.capabilities.contains(capability))
            })
        })
        .take(8)
        .map(|resource| {
            json!({
                "id": resource.id.as_str(),
                "label": resource.label,
                "kind": resource.kind.as_str(),
                "capabilities": resource.capabilities,
            })
        })
        .collect()
}

fn rewrite_required_targets(schema: &mut Value) {
    let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) else {
        return;
    };
    let mut rewritten = Vec::with_capacity(required.len());
    for item in required.drain(..) {
        let item = match item.as_str() {
            Some(field) if PROVIDER_TARGET_FIELDS.contains(&field) => json!("target"),
            _ => item,
        };
        if !rewritten.contains(&item) {
            rewritten.push(item);
        }
    }
    *required = rewritten;
}

fn default_target_property() -> Value {
    json!({
        "type": "string",
        "description": default_target_description()
    })
}

fn enrich_target_property(target: &mut Value) {
    let Some(target) = target.as_object_mut() else {
        return;
    };
    let description = target
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if description.contains("connections.list_sessions") {
        return;
    }
    let description = if description.is_empty() {
        default_target_description().to_string()
    } else {
        format!("{description} {}", default_target_description())
    };
    target.insert("description".to_string(), Value::String(description));
}

fn default_target_description() -> &'static str {
    "Target resource. Call `connections.list_sessions` when needed and copy an exact `id` from its result; labels and aliases are accepted only when they resolve unambiguously."
}
