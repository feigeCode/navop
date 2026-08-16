use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::http::{FieldType, KeyValue, RawLanguage, RequestMethod};
use crate::request_store::{ApiEnvironment, ApiStore, StoredFolder, StoredRequest};

pub const SWAGGER_METHODS: [(&str, RequestMethod); 7] = [
    ("get", RequestMethod::Get),
    ("post", RequestMethod::Post),
    ("put", RequestMethod::Put),
    ("delete", RequestMethod::Delete),
    ("patch", RequestMethod::Patch),
    ("head", RequestMethod::Head),
    ("options", RequestMethod::Options),
];

pub const OPENAPI_METHODS: [(&str, RequestMethod); 8] = [
    ("get", RequestMethod::Get),
    ("post", RequestMethod::Post),
    ("put", RequestMethod::Put),
    ("delete", RequestMethod::Delete),
    ("patch", RequestMethod::Patch),
    ("head", RequestMethod::Head),
    ("options", RequestMethod::Options),
    ("trace", RequestMethod::Trace),
];

pub fn environment(name: &str, server: Option<String>) -> Option<ApiEnvironment> {
    server.map(|value| ApiEnvironment {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: name.to_string(),
        variables: vec![KeyValue::new("baseUrl", value)],
    })
}

pub fn parameter_value(parameter: &Value) -> String {
    parameter
        .get("example")
        .or_else(|| parameter.pointer("/schema/example"))
        .or_else(|| parameter.get("default"))
        .or_else(|| parameter.pointer("/schema/default"))
        .map(value_string)
        .unwrap_or_default()
}

pub fn value_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub fn resolved<'a>(root: &'a Value, value: &'a Value) -> &'a Value {
    const MAX_REFERENCE_DEPTH: usize = 32;
    let mut current = value;
    for _ in 0..MAX_REFERENCE_DEPTH {
        let Some(reference) = current["$ref"].as_str() else {
            return current;
        };
        let Some(pointer) = reference.strip_prefix('#') else {
            return current;
        };
        let Some(next) = root.pointer(pointer) else {
            return current;
        };
        current = next;
    }
    current
}

pub fn combined_parameters(root: &Value, path: &Value, operation: &Value) -> Vec<Value> {
    let mut parameters = BTreeMap::new();
    for parameter in parameter_values(root, path) {
        parameters.insert(parameter_key(&parameter), parameter);
    }
    for parameter in parameter_values(root, operation) {
        parameters.insert(parameter_key(&parameter), parameter);
    }
    parameters.into_values().collect()
}

fn parameter_values(root: &Value, owner: &Value) -> Vec<Value> {
    owner["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|value| resolved(root, value).clone())
        .collect()
}

fn parameter_key(parameter: &Value) -> String {
    format!(
        "{}:{}",
        parameter["in"].as_str().unwrap_or_default(),
        parameter["name"].as_str().unwrap_or_default()
    )
}

pub fn push_parameter(request: &mut StoredRequest, parameter: &Value) {
    let Some(name) = parameter["name"].as_str() else {
        return;
    };
    let row = KeyValue::new(name, parameter_value(parameter));
    match parameter["in"].as_str() {
        Some("path") => request.path_vars.push(row),
        Some("query") => request.params.push(row),
        Some("header") => request.header_rows.push(row),
        Some("cookie") => request.cookies.push(row),
        _ => {}
    }
}

pub fn sync_headers(request: &mut StoredRequest) {
    request.headers = request
        .header_rows
        .iter()
        .filter(|row| row.enabled && !row.key.is_empty())
        .map(|row| format!("{}: {}", row.key, row.value))
        .collect::<Vec<_>>()
        .join("\n");
}

pub fn rows_from_schema(schema: &Value) -> Vec<KeyValue> {
    schema["properties"]
        .as_object()
        .into_iter()
        .flat_map(|properties| properties.iter())
        .map(|(name, property)| {
            let mut row = KeyValue::new(
                name,
                property
                    .get("example")
                    .or_else(|| property.get("default"))
                    .map(value_string)
                    .unwrap_or_default(),
            );
            if property["format"].as_str() == Some("binary")
                || property["type"].as_str() == Some("file")
            {
                row.field_type = FieldType::File;
            }
            row
        })
        .collect()
}

pub fn raw_language(content_type: &str) -> RawLanguage {
    if content_type.contains("xml") {
        RawLanguage::Xml
    } else if content_type.contains("html") {
        RawLanguage::Html
    } else if content_type.contains("javascript") {
        RawLanguage::Javascript
    } else if content_type.contains("json") {
        RawLanguage::Json
    } else {
        RawLanguage::Text
    }
}

pub fn raw_example(media: &Value, language: RawLanguage) -> String {
    let example = media_example(media);
    match example {
        Some(Value::String(value)) if language == RawLanguage::Json => {
            serde_json::to_string(value).unwrap_or_default()
        }
        Some(Value::String(value)) => value.clone(),
        Some(value) => serde_json::to_string_pretty(value).unwrap_or_default(),
        None => String::new(),
    }
}

fn media_example(media: &Value) -> Option<&Value> {
    media
        .get("example")
        .or_else(|| media.pointer("/schema/example"))
        .or_else(|| {
            media["examples"]
                .as_object()?
                .values()
                .find_map(|example| example.get("value"))
        })
}

pub fn folder_for_tag(
    folders: &mut Vec<StoredFolder>,
    ids: &mut BTreeMap<String, String>,
    tag: Option<&str>,
) -> Option<String> {
    let tag = tag?.trim();
    if tag.is_empty() {
        return None;
    }
    if let Some(id) = ids.get(tag) {
        return Some(id.clone());
    }
    let folder = StoredFolder::new(tag, None);
    let id = folder.id.clone();
    ids.insert(tag.to_string(), id.clone());
    folders.push(folder);
    Some(id)
}

pub fn folder_name<'a>(store: &'a ApiStore, request: &StoredRequest) -> Option<&'a str> {
    let id = request.folder_id.as_deref()?;
    store
        .folders
        .iter()
        .find(|folder| folder.id == id)
        .map(|folder| folder.name.as_str())
}

pub fn active_server(store: &ApiStore) -> Option<String> {
    let environment = store
        .active_environment_id
        .as_deref()
        .and_then(|id| store.environments.iter().find(|env| env.id == id))
        .or_else(|| store.environments.first())?;
    environment
        .variables
        .iter()
        .find(|row| row.enabled && row.key == "baseUrl")
        .or_else(|| {
            environment
                .variables
                .iter()
                .find(|row| row.enabled && row.value.starts_with("http"))
        })
        .map(|row| row.value.clone())
}

pub fn operation_id(name: &str, method: RequestMethod) -> String {
    let mut result = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    while result.contains("__") {
        result = result.replace("__", "_");
    }
    let result = result.trim_matches('_');
    if result.is_empty() {
        method.label().to_ascii_lowercase()
    } else {
        result.to_string()
    }
}

pub fn parameter_schema(row: &KeyValue) -> Value {
    let mut schema = json!({"type": "string"});
    if !row.value.is_empty() {
        schema["example"] = json!(row.value);
    }
    schema
}

pub fn object_schema(rows: &[KeyValue]) -> Value {
    let properties = rows
        .iter()
        .filter(|row| row.enabled && !row.key.is_empty())
        .map(|row| {
            let value = if row.field_type == FieldType::File {
                json!({"type": "string", "format": "binary"})
            } else {
                parameter_schema(row)
            };
            (row.key.clone(), value)
        })
        .collect::<Map<_, _>>();
    Value::Object(Map::from_iter([
        ("type".into(), json!("object")),
        ("properties".into(), Value::Object(properties)),
    ]))
}

pub fn example_schema(example: &Value) -> Value {
    let schema_type = match example {
        Value::Null => "string",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    json!({"type": schema_type})
}
