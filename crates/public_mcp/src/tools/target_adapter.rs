use rmcp::{ErrorData as McpError, model::JsonObject};
use serde_json::{Map, Value, json};

const PROVIDER_TARGET_FIELDS: [&str; 3] = ["connection", "connection_id", "session_id"];

pub(super) fn mcp_target_schema(schema: Value) -> Value {
    let mut schema = schema;
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };
    if let Some(target) = target_property(properties) {
        remove_provider_target_fields(properties);
        properties.entry("target".to_string()).or_insert(target);
    }
    rewrite_required_targets(&mut schema);
    schema
}

pub(super) fn normalize_mcp_arguments(schema: &Value, input: Value) -> Result<Value, McpError> {
    let Value::Object(mut input) = input else {
        return Ok(input);
    };
    reject_provider_target_fields(&input)?;
    if descriptor_has_target(schema) {
        return Ok(Value::Object(input));
    }
    let Some(field) = descriptor_provider_target_field(schema) else {
        return Ok(Value::Object(input));
    };
    if let Some(target) = take_target(&mut input)? {
        input.insert(field.to_string(), Value::String(target));
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
        "description": "Target resource id from the MCP-visible resource set."
    })
}
